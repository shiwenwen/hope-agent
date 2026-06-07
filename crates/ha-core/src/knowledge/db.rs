//! IndexDb — the rebuildable note/chunk/link/tag cache (`index.db`).
//!
//! Connection model mirrors the memory backend: one write connection + a small
//! read-only pool, WAL, with the sqlite-vec extension auto-registered before any
//! connection is opened so every connection sees `vec0`. This DB holds **only**
//! cache rows — deleting it loses nothing (the `.md` files + the
//! `knowledge_bases` registry are the truth source).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::memory::EmbeddingProvider;

use super::chunker::ParsedChunk;
use super::parser::ParsedLink;
use super::resolver::{self, NoteRef};
use super::types::{Backlink, BrokenLink, LinkType, Note, NoteLink};

const READ_POOL_SIZE: usize = 4;

/// All inputs needed to (re)write one note's index in a single transaction.
pub struct NoteIndexInput {
    pub kb_id: String,
    pub rel_path: String,
    pub title: String,
    pub frontmatter_json: Option<String>,
    pub mtime: i64,
    pub size: i64,
    pub content_hash: String,
    pub chunks: Vec<ParsedChunk>,
    /// Per-chunk embedding vectors, aligned with `chunks`. `None` = embedding
    /// disabled (FTS-only). When `Some`, length must equal `chunks.len()`.
    pub chunk_embeddings: Option<Vec<Vec<f32>>>,
    pub embedding_signature: Option<String>,
    pub links: Vec<ParsedLink>,
    pub tags: Vec<String>,
}

/// Index cache backend over `~/.hope-agent/knowledge/index.db`.
pub struct IndexDb {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    reader_idx: AtomicUsize,
    /// Embedding dimension of the active model (0 = none). Drives `note_vec`.
    embedding_dims: AtomicU32,
    /// Active embedding provider (installed at startup + on config change),
    /// shared with — but independent of — the memory backend's embedder.
    embedder: RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    #[allow(dead_code)]
    db_path: PathBuf,
}

impl IndexDb {
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        register_sqlite_vec();

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path).with_context(|| {
            format!("Failed to open knowledge index DB at {}", db_path.display())
        })?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Self::create_schema(&conn)?;

        let mut readers = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            let read_conn = Connection::open_with_flags(
                db_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX
                    | OpenFlags::SQLITE_OPEN_URI,
            )
            .with_context(|| format!("Failed to open read connection at {}", db_path.display()))?;
            read_conn.execute_batch("PRAGMA journal_mode=WAL;")?;
            read_conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
            read_conn.execute_batch("PRAGMA foreign_keys=ON;")?;
            readers.push(Mutex::new(read_conn));
        }

        Ok(Self {
            writer: Mutex::new(conn),
            readers,
            reader_idx: AtomicUsize::new(0),
            embedding_dims: AtomicU32::new(0),
            embedder: RwLock::new(None),
            db_path: db_path.to_path_buf(),
        })
    }

    /// Install the active embedding provider. Sets the dimension + ensures the
    /// `note_vec` table matches (recreating on a dimension change).
    pub fn set_embedder(&self, provider: Arc<dyn EmbeddingProvider>) {
        let dims = provider.dimensions();
        if let Ok(mut w) = self.embedder.write() {
            *w = Some(provider);
        }
        self.set_embedding_dims(dims);
    }

    pub fn clear_embedder(&self) {
        if let Ok(mut w) = self.embedder.write() {
            *w = None;
        }
        self.embedding_dims.store(0, Ordering::Relaxed);
    }

    /// A clone of the active embedding provider, if any.
    pub fn embedder(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embedder.read().ok().and_then(|g| g.clone())
    }

    fn create_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS note (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kb_id TEXT NOT NULL,
                rel_path TEXT NOT NULL,
                title TEXT NOT NULL,
                frontmatter_json TEXT,
                mtime INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                size INTEGER NOT NULL,
                UNIQUE(kb_id, rel_path)
            );
            CREATE INDEX IF NOT EXISTS idx_note_kb ON note(kb_id);
            CREATE INDEX IF NOT EXISTS idx_note_title ON note(kb_id, title);

            CREATE TABLE IF NOT EXISTS note_chunk (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                note_id INTEGER NOT NULL REFERENCES note(id) ON DELETE CASCADE,
                chunk_index INTEGER NOT NULL,
                heading_path TEXT,
                body TEXT NOT NULL,
                start_offset INTEGER NOT NULL,
                end_offset INTEGER NOT NULL,
                start_line INTEGER NOT NULL,
                start_col INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                end_col INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                embedding_signature TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_chunk_note ON note_chunk(note_id);

            CREATE VIRTUAL TABLE IF NOT EXISTS note_chunk_fts USING fts5(
                body,
                content='note_chunk', content_rowid='id',
                tokenize='unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS note_chunk_ai AFTER INSERT ON note_chunk BEGIN
                INSERT INTO note_chunk_fts(rowid, body) VALUES (new.id, new.body);
            END;
            CREATE TRIGGER IF NOT EXISTS note_chunk_ad AFTER DELETE ON note_chunk BEGIN
                INSERT INTO note_chunk_fts(note_chunk_fts, rowid, body)
                VALUES ('delete', old.id, old.body);
            END;
            CREATE TRIGGER IF NOT EXISTS note_chunk_au AFTER UPDATE ON note_chunk BEGIN
                INSERT INTO note_chunk_fts(note_chunk_fts, rowid, body)
                VALUES ('delete', old.id, old.body);
                INSERT INTO note_chunk_fts(rowid, body) VALUES (new.id, new.body);
            END;

            CREATE TABLE IF NOT EXISTS note_link (
                src_note_id INTEGER NOT NULL REFERENCES note(id) ON DELETE CASCADE,
                target_ref TEXT NOT NULL,
                target_note_id INTEGER REFERENCES note(id) ON DELETE SET NULL,
                link_type TEXT NOT NULL,
                anchor TEXT,
                alias TEXT,
                raw_text TEXT NOT NULL,
                src_start_line INTEGER NOT NULL,
                src_start_col INTEGER NOT NULL,
                src_end_line INTEGER NOT NULL,
                src_end_col INTEGER NOT NULL,
                src_heading_path TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_link_src ON note_link(src_note_id);
            CREATE INDEX IF NOT EXISTS idx_link_target ON note_link(target_note_id);

            CREATE TABLE IF NOT EXISTS note_tag (
                note_id INTEGER NOT NULL REFERENCES note(id) ON DELETE CASCADE,
                tag TEXT NOT NULL,
                PRIMARY KEY (note_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_tag ON note_tag(tag);",
        )?;
        Ok(())
    }

    /// Set the embedding dimension (from the active provider) and ensure the
    /// `note_vec` table matches. Drops + recreates on a dimension change.
    pub fn set_embedding_dims(&self, dims: u32) {
        self.embedding_dims.store(dims, Ordering::Relaxed);
        if dims > 0 {
            if let Ok(conn) = self.writer.lock() {
                let _ = ensure_vec_table(&conn, dims);
            }
        }
    }

    pub fn embedding_dims(&self) -> u32 {
        self.embedding_dims.load(Ordering::Relaxed)
    }

    fn read_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        let idx = self.reader_idx.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        for i in 0..self.readers.len() {
            let target = (idx + i) % self.readers.len();
            if let Ok(guard) = self.readers[target].try_lock() {
                return Ok(guard);
            }
        }
        self.readers[idx]
            .lock()
            .map_err(|e| anyhow::anyhow!("Read pool lock poisoned: {e}"))
    }

    fn write_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.writer
            .lock()
            .map_err(|e| anyhow::anyhow!("Writer lock poisoned: {e}"))
    }

    // ── Note reads ─────────────────────────────────────────────

    pub fn get_note_by_rel_path(&self, kb_id: &str, rel_path: &str) -> Result<Option<Note>> {
        let conn = self.read_conn()?;
        let row = conn
            .query_row(
                "SELECT id, kb_id, rel_path, title, frontmatter_json, mtime, content_hash, size
                 FROM note WHERE kb_id = ?1 AND rel_path = ?2",
                params![kb_id, rel_path],
                row_to_note,
            )
            .optional()?;
        Ok(row)
    }

    pub fn get_note_by_id(&self, note_id: i64) -> Result<Option<Note>> {
        let conn = self.read_conn()?;
        let row = conn
            .query_row(
                "SELECT id, kb_id, rel_path, title, frontmatter_json, mtime, content_hash, size
                 FROM note WHERE id = ?1",
                params![note_id],
                row_to_note,
            )
            .optional()?;
        Ok(row)
    }

    /// `(rel_path, mtime, content_hash)` for every note in a KB — drives the
    /// incremental reconcile (compare against on-disk mtime/size).
    pub fn note_index_state(&self, kb_id: &str) -> Result<Vec<(String, i64, String)>> {
        let conn = self.read_conn()?;
        let mut stmt =
            conn.prepare("SELECT rel_path, mtime, content_hash FROM note WHERE kb_id = ?1")?;
        let rows = stmt.query_map(params![kb_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn count_notes(&self, kb_id: &str) -> Result<u32> {
        let conn = self.read_conn()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note WHERE kb_id = ?1",
            params![kb_id],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    /// Minimal note descriptors for the deterministic resolver.
    pub fn note_refs(&self, kb_id: &str) -> Result<Vec<NoteRef>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare("SELECT id, rel_path, title FROM note WHERE kb_id = ?1")?;
        let rows = stmt.query_map(params![kb_id], |r| {
            Ok(NoteRef {
                id: r.get(0)?,
                rel_path: r.get(1)?,
                title: r.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Full note list (metadata) for a KB, ordered by path.
    pub fn list_notes(&self, kb_id: &str) -> Result<Vec<Note>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, kb_id, rel_path, title, frontmatter_json, mtime, content_hash, size
             FROM note WHERE kb_id = ?1 ORDER BY rel_path",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_note)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── Links / tags reads ─────────────────────────────────────

    pub fn outgoing_links(&self, src_note_id: i64) -> Result<Vec<NoteLink>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT src_note_id, target_ref, target_note_id, link_type, anchor, alias, raw_text,
                    src_start_line, src_start_col, src_end_line, src_end_col, src_heading_path
             FROM note_link WHERE src_note_id = ?1
             ORDER BY src_start_line, src_start_col",
        )?;
        let rows = stmt.query_map(params![src_note_id], row_to_link)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Backlinks: notes whose links resolve to `target_note_id`, with enough
    /// context to jump to the exact link occurrence.
    pub fn backlinks(&self, target_note_id: i64) -> Result<Vec<Backlink>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT l.src_note_id, n.rel_path, n.title, l.raw_text,
                    l.src_start_line, l.src_start_col, l.src_heading_path
             FROM note_link l JOIN note n ON n.id = l.src_note_id
             WHERE l.target_note_id = ?1
             ORDER BY n.rel_path, l.src_start_line",
        )?;
        let rows = stmt.query_map(params![target_note_id], |r| {
            Ok(Backlink {
                src_note_id: r.get(0)?,
                src_rel_path: r.get(1)?,
                src_title: r.get(2)?,
                raw_text: r.get(3)?,
                src_start_line: r.get::<_, i64>(4)? as u32,
                src_start_col: r.get::<_, i64>(5)? as u32,
                src_heading_path: r.get::<_, Option<String>>(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Block-level backlinks (Phase 3 G): `[[Note#^block-id]]` references that
    /// resolve to `target_note_id` and carry the matching `^block-id` anchor.
    /// Anchor match is case-insensitive (resolve parity).
    pub fn block_backlinks(&self, target_note_id: i64, block_id: &str) -> Result<Vec<Backlink>> {
        let anchor = format!("^{block_id}");
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT l.src_note_id, n.rel_path, n.title, l.raw_text,
                    l.src_start_line, l.src_start_col, l.src_heading_path
             FROM note_link l JOIN note n ON n.id = l.src_note_id
             WHERE l.target_note_id = ?1 AND l.anchor IS NOT NULL
                   AND l.anchor = ?2 COLLATE NOCASE
             ORDER BY n.rel_path, l.src_start_line",
        )?;
        let rows = stmt.query_map(params![target_note_id, anchor], |r| {
            Ok(Backlink {
                src_note_id: r.get(0)?,
                src_rel_path: r.get(1)?,
                src_title: r.get(2)?,
                raw_text: r.get(3)?,
                src_start_line: r.get::<_, i64>(4)? as u32,
                src_start_col: r.get::<_, i64>(5)? as u32,
                src_heading_path: r.get::<_, Option<String>>(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn tags_for_note(&self, note_id: i64) -> Result<Vec<String>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare("SELECT tag FROM note_tag WHERE note_id = ?1 ORDER BY tag")?;
        let rows = stmt.query_map(params![note_id], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Notes carrying `tag` within the given KBs.
    pub fn notes_by_tag(&self, kb_ids: &[String], tag: &str) -> Result<Vec<Note>> {
        if kb_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.read_conn()?;
        let placeholders = sql_in_placeholders(kb_ids.len(), 2);
        let sql = format!(
            "SELECT DISTINCT n.id, n.kb_id, n.rel_path, n.title, n.frontmatter_json, n.mtime,
                    n.content_hash, n.size
             FROM note n JOIN note_tag t ON t.note_id = n.id
             WHERE t.tag = ?1 AND n.kb_id IN ({placeholders})
             ORDER BY n.rel_path"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut binds: Vec<&dyn rusqlite::types::ToSql> = vec![&tag];
        for k in kb_ids {
            binds.push(k);
        }
        let rows = stmt.query_map(binds.as_slice(), row_to_note)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Distinct tags across the given KBs, with counts.
    pub fn all_tags(&self, kb_ids: &[String]) -> Result<Vec<(String, u32)>> {
        if kb_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.read_conn()?;
        let placeholders = sql_in_placeholders(kb_ids.len(), 1);
        let sql = format!(
            "SELECT t.tag, COUNT(*) c
             FROM note_tag t JOIN note n ON n.id = t.note_id
             WHERE n.kb_id IN ({placeholders})
             GROUP BY t.tag ORDER BY c DESC, t.tag"
        );
        let mut stmt = conn.prepare(&sql)?;
        let binds: Vec<&dyn rusqlite::types::ToSql> = kb_ids
            .iter()
            .map(|k| k as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(binds.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── Search primitives (FTS + vec) ──────────────────────────

    /// chunk-level FTS5 BM25 search. Returns `(chunk_id, note_id, rank)`
    /// (lower rank = better) ordered best-first.
    pub fn fts_search(
        &self,
        kb_ids: &[String],
        query: &str,
        limit: usize,
    ) -> Result<Vec<(i64, i64, f64)>> {
        if kb_ids.is_empty() {
            return Ok(Vec::new());
        }
        let Some(match_query) = build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.read_conn()?;
        let placeholders = sql_in_placeholders(kb_ids.len(), 3);
        let sql = format!(
            "SELECT f.rowid, nc.note_id, f.rank
             FROM note_chunk_fts f
             JOIN note_chunk nc ON nc.id = f.rowid
             JOIN note n ON n.id = nc.note_id
             WHERE note_chunk_fts MATCH ?1 AND n.kb_id IN ({placeholders})
             ORDER BY f.rank LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let limit_i = limit as i64;
        let mut binds: Vec<&dyn rusqlite::types::ToSql> = vec![&match_query, &limit_i];
        for k in kb_ids {
            binds.push(k);
        }
        let rows = stmt.query_map(binds.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// chunk-level vector KNN. Returns `(chunk_id, note_id, distance)`
    /// (lower distance = closer), signature-filtered so stale-model vectors
    /// never mix in.
    pub fn vec_search(
        &self,
        kb_ids: &[String],
        query_embedding: &[f32],
        signature: &str,
        limit: usize,
    ) -> Result<Vec<(i64, i64, f64)>> {
        if kb_ids.is_empty() || self.embedding_dims() == 0 {
            return Ok(Vec::new());
        }
        let conn = self.read_conn()?;
        if !vec_table_exists(&conn)? {
            return Ok(Vec::new());
        }
        let bytes: Vec<u8> = query_embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let placeholders = sql_in_placeholders(kb_ids.len(), 4);
        // Mirror the memory backend: KNN with a `rowid IN (subquery)` filter so
        // sqlite-vec accepts the extra constraints.
        let sql = format!(
            "SELECT rowid, distance FROM note_vec
             WHERE embedding MATCH ?1
               AND rowid IN (
                   SELECT nc.id FROM note_chunk nc JOIN note n ON n.id = nc.note_id
                   WHERE nc.embedding_signature = ?3 AND n.kb_id IN ({placeholders})
               )
             ORDER BY distance LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let limit_i = limit as i64;
        let mut binds: Vec<&dyn rusqlite::types::ToSql> = vec![&bytes, &limit_i, &signature];
        for k in kb_ids {
            binds.push(k);
        }
        let hits: Vec<(i64, f64)> = stmt
            .query_map(binds.as_slice(), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        // Resolve note_id for each chunk hit.
        let ids: Vec<i64> = hits.iter().map(|(id, _)| *id).collect();
        let note_map = self.chunk_note_ids(&conn, &ids)?;
        Ok(hits
            .into_iter()
            .filter_map(|(cid, dist)| note_map.get(&cid).map(|nid| (cid, *nid, dist)))
            .collect())
    }

    fn chunk_note_ids(
        &self,
        conn: &Connection,
        chunk_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, i64>> {
        if chunk_ids.is_empty() {
            return Ok(Default::default());
        }
        let placeholders = sql_in_placeholders(chunk_ids.len(), 1);
        let sql = format!("SELECT id, note_id FROM note_chunk WHERE id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let binds: Vec<&dyn rusqlite::types::ToSql> = chunk_ids
            .iter()
            .map(|i| i as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(binds.as_slice(), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let (a, b) = r?;
            map.insert(a, b);
        }
        Ok(map)
    }

    /// `(note_id, rel_path, title)` for a set of note ids (search aggregation).
    pub fn notes_for_ids(
        &self,
        note_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, (String, String, String)>> {
        if note_ids.is_empty() {
            return Ok(Default::default());
        }
        let conn = self.read_conn()?;
        let placeholders = sql_in_placeholders(note_ids.len(), 1);
        let sql =
            format!("SELECT id, kb_id, rel_path, title FROM note WHERE id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let binds: Vec<&dyn rusqlite::types::ToSql> = note_ids
            .iter()
            .map(|i| i as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(binds.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                (
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ),
            ))
        })?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let (id, tup) = r?;
            map.insert(id, tup);
        }
        Ok(map)
    }

    /// Best chunk (snippet + heading + line) per note for a set of chunk ids.
    pub fn chunk_snippets(
        &self,
        chunk_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, (String, Option<String>, u32)>> {
        if chunk_ids.is_empty() {
            return Ok(Default::default());
        }
        let conn = self.read_conn()?;
        let placeholders = sql_in_placeholders(chunk_ids.len(), 1);
        let sql = format!(
            "SELECT id, body, heading_path, start_line FROM note_chunk WHERE id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let binds: Vec<&dyn rusqlite::types::ToSql> = chunk_ids
            .iter()
            .map(|i| i as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(binds.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                (
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)? as u32,
                ),
            ))
        })?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let (id, tup) = r?;
            map.insert(id, tup);
        }
        Ok(map)
    }

    // ── Writes ─────────────────────────────────────────────────

    /// Transactionally (re)write one note's full index: note row + chunks (with
    /// FTS via triggers + vec rows) + tags + outgoing links. Inbound links to
    /// this note are preserved (the note id is stable across re-index). Returns
    /// the note id. Call [`reresolve_kb_links`] afterward to (re)resolve link
    /// targets across the KB.
    pub fn replace_note_index(&self, input: NoteIndexInput) -> Result<i64> {
        if let Some(embs) = &input.chunk_embeddings {
            anyhow::ensure!(
                embs.len() == input.chunks.len(),
                "chunk_embeddings length must match chunks"
            );
        }
        let dims = self.embedding_dims();
        let mut conn = self.write_conn()?;
        if dims > 0 {
            let _ = ensure_vec_table(&conn, dims);
        }
        let tx = conn.transaction()?;

        // Upsert note row.
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM note WHERE kb_id = ?1 AND rel_path = ?2",
                params![input.kb_id, input.rel_path],
                |r| r.get(0),
            )
            .optional()?;

        let note_id = match existing {
            Some(id) => {
                tx.execute(
                    "UPDATE note SET title = ?2, frontmatter_json = ?3, mtime = ?4,
                         content_hash = ?5, size = ?6 WHERE id = ?1",
                    params![
                        id,
                        input.title,
                        input.frontmatter_json,
                        input.mtime,
                        input.content_hash,
                        input.size
                    ],
                )?;
                // Clear this note's chunks (+ their vec rows), tags, outgoing links.
                delete_chunks_for_note(&tx, id)?;
                tx.execute("DELETE FROM note_tag WHERE note_id = ?1", params![id])?;
                tx.execute("DELETE FROM note_link WHERE src_note_id = ?1", params![id])?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO note (kb_id, rel_path, title, frontmatter_json, mtime,
                         content_hash, size)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        input.kb_id,
                        input.rel_path,
                        input.title,
                        input.frontmatter_json,
                        input.mtime,
                        input.content_hash,
                        input.size
                    ],
                )?;
                tx.last_insert_rowid()
            }
        };

        // Insert chunks (FTS synced by trigger) + vec rows.
        for (i, ch) in input.chunks.iter().enumerate() {
            tx.execute(
                "INSERT INTO note_chunk
                    (note_id, chunk_index, heading_path, body, start_offset, end_offset,
                     start_line, start_col, end_line, end_col, content_hash, embedding_signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    note_id,
                    ch.chunk_index,
                    ch.heading_path,
                    ch.body,
                    ch.start_offset,
                    ch.end_offset,
                    ch.start_line,
                    ch.start_col,
                    ch.end_line,
                    ch.end_col,
                    ch.content_hash,
                    input.embedding_signature,
                ],
            )?;
            let chunk_id = tx.last_insert_rowid();
            if let (Some(embs), true) = (&input.chunk_embeddings, dims > 0) {
                let emb = &embs[i];
                if emb.len() as u32 == dims {
                    let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                    let _ = tx.execute(
                        "INSERT INTO note_vec(rowid, embedding) VALUES (?1, ?2)",
                        params![chunk_id, bytes],
                    );
                }
            }
        }

        // Insert tags.
        for tag in &input.tags {
            tx.execute(
                "INSERT OR IGNORE INTO note_tag(note_id, tag) VALUES (?1, ?2)",
                params![note_id, tag],
            )?;
        }

        // Insert outgoing links (targets resolved later by reresolve_kb_links).
        for l in &input.links {
            tx.execute(
                "INSERT INTO note_link
                    (src_note_id, target_ref, target_note_id, link_type, anchor, alias, raw_text,
                     src_start_line, src_start_col, src_end_line, src_end_col, src_heading_path)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    note_id,
                    l.target_ref,
                    l.link_type.as_str(),
                    l.anchor,
                    l.alias,
                    l.raw_text,
                    l.start.line,
                    l.start.col,
                    l.end.line,
                    l.end.col,
                    l.heading_path,
                ],
            )?;
        }

        tx.commit()?;
        Ok(note_id)
    }

    /// Remove a single note from the index (by rel_path). Inbound links become
    /// dangling (target set NULL). Returns true if a row was removed.
    pub fn delete_note(&self, kb_id: &str, rel_path: &str) -> Result<bool> {
        let mut conn = self.write_conn()?;
        let tx = conn.transaction()?;
        let id: Option<i64> = tx
            .query_row(
                "SELECT id FROM note WHERE kb_id = ?1 AND rel_path = ?2",
                params![kb_id, rel_path],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(false);
        };
        delete_chunks_for_note(&tx, id)?;
        tx.execute("DELETE FROM note_tag WHERE note_id = ?1", params![id])?;
        tx.execute("DELETE FROM note_link WHERE src_note_id = ?1", params![id])?;
        // Inbound links to this note become dangling.
        tx.execute(
            "UPDATE note_link SET target_note_id = NULL WHERE target_note_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM note WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Remove every index row for a KB (used by `delete_kb_cascade` + reindex).
    pub fn prune_kb(&self, kb_id: &str) -> Result<()> {
        let mut conn = self.write_conn()?;
        let tx = conn.transaction()?;
        // Collect chunk ids to clear vec rows.
        let chunk_ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT nc.id FROM note_chunk nc JOIN note n ON n.id = nc.note_id
                 WHERE n.kb_id = ?1",
            )?;
            let rows = stmt.query_map(params![kb_id], |r| r.get::<_, i64>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for cid in &chunk_ids {
            let _ = tx.execute("DELETE FROM note_vec WHERE rowid = ?1", params![cid]);
        }
        // Deleting note rows cascades to chunks/tags/links via FK; but to be sure
        // FTS stays consistent we delete chunks directly first (fires triggers).
        tx.execute(
            "DELETE FROM note_chunk WHERE note_id IN (SELECT id FROM note WHERE kb_id = ?1)",
            params![kb_id],
        )?;
        tx.execute(
            "DELETE FROM note_tag WHERE note_id IN (SELECT id FROM note WHERE kb_id = ?1)",
            params![kb_id],
        )?;
        tx.execute(
            "DELETE FROM note_link WHERE src_note_id IN (SELECT id FROM note WHERE kb_id = ?1)",
            params![kb_id],
        )?;
        tx.execute("DELETE FROM note WHERE kb_id = ?1", params![kb_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Re-resolve every link target in a KB against the current note set
    /// (deterministic, design #8). Run after any note add / delete so inbound
    /// links flip broken↔resolved correctly.
    pub fn reresolve_kb_links(&self, kb_id: &str) -> Result<()> {
        let notes = self.note_refs(kb_id)?;
        let mut conn = self.write_conn()?;
        let tx = conn.transaction()?;
        // (rowid, target_ref) for every link whose source is in this KB.
        let links: Vec<(i64, String)> = {
            let mut stmt = tx.prepare(
                "SELECT l.rowid, l.target_ref
                 FROM note_link l JOIN note n ON n.id = l.src_note_id
                 WHERE n.kb_id = ?1",
            )?;
            let rows = stmt.query_map(params![kb_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (rowid, target_ref) in links {
            let target = resolver::resolve(&target_ref, &notes);
            tx.execute(
                "UPDATE note_link SET target_note_id = ?2 WHERE rowid = ?1",
                params![rowid, target],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Count broken (dangling) links in a KB.
    pub fn count_broken_links(&self, kb_id: &str) -> Result<u32> {
        let conn = self.read_conn()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_link l JOIN note n ON n.id = l.src_note_id
             WHERE n.kb_id = ?1 AND l.target_note_id IS NULL",
            params![kb_id],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    /// Every broken (dangling) link in a KB, with the source note + exact
    /// occurrence + unresolved `target_ref` (so the UI can offer "create note").
    pub fn list_broken_links(&self, kb_id: &str) -> Result<Vec<BrokenLink>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT l.src_note_id, n.rel_path, n.title, l.target_ref, l.raw_text,
                    l.src_start_line, l.src_start_col, l.src_heading_path
             FROM note_link l JOIN note n ON n.id = l.src_note_id
             WHERE n.kb_id = ?1 AND l.target_note_id IS NULL
             ORDER BY n.rel_path, l.src_start_line, l.src_start_col",
        )?;
        let rows = stmt.query_map(params![kb_id], |r| {
            Ok(BrokenLink {
                src_note_id: r.get(0)?,
                src_rel_path: r.get(1)?,
                src_title: r.get(2)?,
                target_ref: r.get(3)?,
                raw_text: r.get(4)?,
                src_start_line: r.get::<_, i64>(5)? as u32,
                src_start_col: r.get::<_, i64>(6)? as u32,
                src_heading_path: r.get::<_, Option<String>>(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Every **resolved** link edge `(src_note_id, target_note_id)` in a KB —
    /// the raw material for the link graph (WS1). Broken links (NULL target) are
    /// excluded; parallel edges are returned as-is (the graph builder collapses
    /// them). Both endpoints are notes in `kb_id` (wikilinks are intra-KB).
    pub fn all_resolved_links(&self, kb_id: &str) -> Result<Vec<(i64, i64)>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT l.src_note_id, l.target_note_id
             FROM note_link l JOIN note n ON n.id = l.src_note_id
             WHERE n.kb_id = ?1 AND l.target_note_id IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![kb_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Orphan notes: no resolved inbound link **and** no resolved outbound link
    /// (broken links don't count as a connection). The "islands" in the graph.
    pub fn list_orphan_notes(&self, kb_id: &str) -> Result<Vec<Note>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, kb_id, rel_path, title, frontmatter_json, mtime, content_hash, size
             FROM note n
             WHERE n.kb_id = ?1
               AND NOT EXISTS (SELECT 1 FROM note_link l WHERE l.target_note_id = n.id)
               AND NOT EXISTS (
                   SELECT 1 FROM note_link l
                   WHERE l.src_note_id = n.id AND l.target_note_id IS NOT NULL
               )
             ORDER BY n.rel_path",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_note)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

// ── Free helpers ────────────────────────────────────────────────

fn register_sqlite_vec() {
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    }
}

fn ensure_vec_table(conn: &Connection, dims: u32) -> Result<()> {
    let existing_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'note_vec'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let expected = format!("float[{}]", dims);
    if let Some(sql) = existing_sql {
        if !sql.contains(&expected) {
            conn.execute_batch("DROP TABLE IF EXISTS note_vec;")?;
        }
    }
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS note_vec USING vec0(rowid INTEGER PRIMARY KEY, embedding float[{}])",
        dims
    );
    conn.execute_batch(&sql)?;
    Ok(())
}

fn vec_table_exists(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_vec'",
        [],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn delete_chunks_for_note(tx: &rusqlite::Transaction, note_id: i64) -> Result<()> {
    let chunk_ids: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM note_chunk WHERE note_id = ?1")?;
        let rows = stmt.query_map(params![note_id], |r| r.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for cid in &chunk_ids {
        let _ = tx.execute("DELETE FROM note_vec WHERE rowid = ?1", params![cid]);
    }
    tx.execute(
        "DELETE FROM note_chunk WHERE note_id = ?1",
        params![note_id],
    )?;
    Ok(())
}

fn row_to_note(row: &rusqlite::Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        kb_id: row.get(1)?,
        rel_path: row.get(2)?,
        title: row.get(3)?,
        frontmatter_json: row.get::<_, Option<String>>(4)?,
        mtime: row.get(5)?,
        content_hash: row.get(6)?,
        size: row.get(7)?,
    })
}

fn row_to_link(row: &rusqlite::Row) -> rusqlite::Result<NoteLink> {
    let link_type: String = row.get(3)?;
    Ok(NoteLink {
        src_note_id: row.get(0)?,
        target_ref: row.get(1)?,
        target_note_id: row.get::<_, Option<i64>>(2)?,
        link_type: LinkType::from_str_lenient(&link_type),
        anchor: row.get::<_, Option<String>>(4)?,
        alias: row.get::<_, Option<String>>(5)?,
        raw_text: row.get(6)?,
        src_start_line: row.get::<_, i64>(7)? as u32,
        src_start_col: row.get::<_, i64>(8)? as u32,
        src_end_line: row.get::<_, i64>(9)? as u32,
        src_end_col: row.get::<_, i64>(10)? as u32,
        src_heading_path: row.get::<_, Option<String>>(11)?,
    })
}

/// `?{start}, ?{start+1}, ...` for an `IN (...)` clause of `n` items.
fn sql_in_placeholders(n: usize, start: usize) -> String {
    (0..n)
        .map(|i| format!("?{}", start + i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build an FTS5 MATCH string: drop very short tokens, clean each to
/// `[alphanumeric _ -]` (preserving CJK), double-quote, OR-join. Returns `None`
/// if nothing usable remains (so we skip an unbounded scan).
fn build_fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split(|c: char| c.is_whitespace())
        .filter_map(|w| {
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if clean.chars().count() <= 1 {
                None
            } else {
                Some(format!("\"{}\"", clean))
            }
        })
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{chunker, parser};
    use tempfile::tempdir;

    fn input(kb: &str, rel: &str, full: &str) -> NoteIndexInput {
        let parsed = parser::parse_document(full);
        let chunks = chunker::chunk(full, &parsed, &chunker::ChunkConfig::default());
        NoteIndexInput {
            kb_id: kb.into(),
            rel_path: rel.into(),
            title: parsed.title.clone().unwrap_or_else(|| rel.into()),
            frontmatter_json: parsed.frontmatter_json,
            mtime: 1,
            size: full.len() as i64,
            content_hash: crate::knowledge::blake3_hex(full.as_bytes()),
            chunks,
            chunk_embeddings: None,
            embedding_signature: None,
            links: parsed.links,
            tags: parsed.tags,
        }
    }

    #[test]
    fn index_search_backlinks_resolve() {
        let dir = tempdir().unwrap();
        let db = IndexDb::open(&dir.path().join("index.db")).unwrap();
        let kb = "kb1";

        db.replace_note_index(input(kb, "A.md", "# A\n\nSee [[B]] for rust embeddings.\n"))
            .unwrap();
        db.replace_note_index(input(kb, "B.md", "# B\n\nrust embeddings content here.\n"))
            .unwrap();
        db.reresolve_kb_links(kb).unwrap();

        // FTS finds both notes for "rust".
        let hits = db
            .fts_search(&[kb.to_string()], "rust embeddings", 10)
            .unwrap();
        assert!(!hits.is_empty(), "fts should match");

        // Backlinks: B is linked from A.
        let b = db.get_note_by_rel_path(kb, "B.md").unwrap().unwrap();
        let bl = db.backlinks(b.id).unwrap();
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].src_rel_path, "A.md");
        assert_eq!(db.count_broken_links(kb).unwrap(), 0);

        // Delete B → A's link to it becomes broken.
        db.delete_note(kb, "B.md").unwrap();
        db.reresolve_kb_links(kb).unwrap();
        assert_eq!(db.count_broken_links(kb).unwrap(), 1);
        assert_eq!(db.count_notes(kb).unwrap(), 1);
    }

    #[test]
    fn broken_links_and_orphans() {
        let dir = tempdir().unwrap();
        let db = IndexDb::open(&dir.path().join("index.db")).unwrap();
        let kb = "kb1";
        // A → B (resolves); C → [[missing]] (broken); D has no links.
        db.replace_note_index(input(kb, "A.md", "# A\n\nSee [[B]].\n"))
            .unwrap();
        db.replace_note_index(input(kb, "B.md", "# B\n\nbody.\n"))
            .unwrap();
        db.replace_note_index(input(kb, "C.md", "# C\n\nSee [[missing]].\n"))
            .unwrap();
        db.replace_note_index(input(kb, "D.md", "# D\n\njust text.\n"))
            .unwrap();
        db.reresolve_kb_links(kb).unwrap();

        let broken = db.list_broken_links(kb).unwrap();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].src_rel_path, "C.md");
        assert_eq!(broken[0].target_ref, "missing");

        // Orphans: C (only a broken outbound) + D (nothing). A/B are connected.
        let orphans = db.list_orphan_notes(kb).unwrap();
        let paths: Vec<&str> = orphans.iter().map(|n| n.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["C.md", "D.md"]);
    }

    #[test]
    fn tags_indexed_and_queryable() {
        let dir = tempdir().unwrap();
        let db = IndexDb::open(&dir.path().join("index.db")).unwrap();
        let kb = "kb1";
        db.replace_note_index(input(kb, "n.md", "# N\n\nbody #rust #pkm\n"))
            .unwrap();
        let by_tag = db.notes_by_tag(&[kb.to_string()], "rust").unwrap();
        assert_eq!(by_tag.len(), 1);
        let all = db.all_tags(&[kb.to_string()]).unwrap();
        assert!(all.iter().any(|(t, _)| t == "pkm"));
    }
}

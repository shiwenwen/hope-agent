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
    /// DB path for opening new connections
    pub(crate) db_path: std::path::PathBuf,
}

impl SqliteMemoryBackend {
    /// Open (or create) the memory database with sqlite-vec extension.
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        // Register sqlite-vec extension before opening connection
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open memory DB at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

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
            CREATE INDEX IF NOT EXISTS idx_memories_updated
                ON memories(updated_at DESC);

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
                embedding BLOB NOT NULL,
                dimensions INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (hash, provider, model)
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

        // Load provider/model info from config for cache key
        let store = crate::provider::load_store().ok()?;
        let provider_key = format!("{:?}", store.embedding.provider_type);
        let model_key = store.embedding.api_model.clone().unwrap_or_default();

        // Check cache (read-only)
        if let Ok(conn) = self.read_conn() {
            let cached: Option<Vec<u8>> = conn.query_row(
                "SELECT embedding FROM embedding_cache WHERE hash = ?1 AND provider = ?2 AND model = ?3",
                params![hash_str, provider_key, model_key],
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
                "INSERT OR REPLACE INTO embedding_cache (hash, provider, model, embedding, dimensions, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
                params![hash_str, provider_key, model_key, emb_bytes, dims],
            );

            // Prune cache if over limit
            if cache_cfg.max_entries > 0 {
                let count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
                    .unwrap_or(0);
                if count as usize > cache_cfg.max_entries {
                    let to_delete =
                        (count as usize - cache_cfg.max_entries + cache_cfg.max_entries / 10) as i64;
                    let _ = conn.execute(
                        "DELETE FROM embedding_cache WHERE rowid IN (SELECT rowid FROM embedding_cache ORDER BY created_at ASC LIMIT ?1)",
                        params![to_delete],
                    );
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
        let conn = self.write_conn()?;
        let dims = self
            .embedding_dims
            .load(std::sync::atomic::Ordering::Relaxed);
        if dims == 0 {
            return Err(anyhow::anyhow!("No embedding provider configured"));
        }
        let _ = self.ensure_vec_table(&conn, dims);

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
                            let mut count = 0usize;
                            // Use a transaction for batch embedding updates (significant perf improvement)
                            let _ = conn.execute_batch("BEGIN");
                            for (id_str, emb) in &results {
                                let id: i64 = id_str.parse().unwrap_or(0);
                                if id == 0 {
                                    continue;
                                }
                                let emb_bytes: Vec<u8> =
                                    emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                                let _ = conn.execute(
                                    "UPDATE memories SET embedding = ?1 WHERE id = ?2",
                                    params![emb_bytes, id],
                                );
                                let _ = conn.execute(
                                    "DELETE FROM memories_vec WHERE rowid = ?1",
                                    params![id],
                                );
                                let _ = conn.execute(
                                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                                    params![id, emb_bytes],
                                );
                                count += 1;
                            }

                            // Handle multimodal entries with synchronous fallback
                            for entry in entries.iter().filter(|e| e.attachment_path.is_some()) {
                                if let Some(emb) = self.generate_multimodal_embedding(
                                    &entry.content,
                                    entry.attachment_path.as_deref().unwrap_or(""),
                                    entry.attachment_mime.as_deref().unwrap_or(""),
                                ) {
                                    let emb_bytes: Vec<u8> =
                                        emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                                    let _ = conn.execute(
                                        "UPDATE memories SET embedding = ?1 WHERE id = ?2",
                                        params![emb_bytes, entry.id],
                                    );
                                    let _ = conn.execute(
                                        "DELETE FROM memories_vec WHERE rowid = ?1",
                                        params![entry.id],
                                    );
                                    let _ = conn.execute("INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)", params![entry.id, emb_bytes]);
                                    count += 1;
                                }
                            }
                            let _ = conn.execute_batch("COMMIT");

                            return Ok(count);
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
        let mut count = 0usize;
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
                conn.execute(
                    "UPDATE memories SET embedding = ?1 WHERE id = ?2",
                    params![emb_bytes, entry.id],
                )?;
                let _ = conn.execute(
                    "DELETE FROM memories_vec WHERE rowid = ?1",
                    params![entry.id],
                );
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![entry.id, emb_bytes],
                );
                count += 1;
            }
        }
        Ok(count)
    }
}

// ── Helper: scope -> SQL conditions ──────────────────────────────

/// Returns (where_clause, params) for scope filtering.
/// `agent_id` is an optional shorthand that means "global + this agent".
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
    let tags_json: String = row.get("tags")?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

    let scope = if scope_type == "agent" {
        MemoryScope::Agent {
            id: scope_agent_id.unwrap_or_default(),
        }
    } else {
        MemoryScope::Global
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
        attachment_path: row.get("attachment_path").ok().flatten(),
        attachment_mime: row.get("attachment_mime").ok().flatten(),
    })
}

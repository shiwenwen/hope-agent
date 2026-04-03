use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::sync::{Arc, Mutex};

use super::helpers::{
    expand_query, load_dedup_config, load_hybrid_search_config, load_temporal_decay_config,
};
use super::traits::{EmbeddingProvider, MemoryBackend};
use super::types::*;

/// Number of read-only connections in the pool.
const READ_POOL_SIZE: usize = 4;

// ── Prompt Injection Protection ─────────────────────────────────

/// Sanitize memory content before injecting into system prompt.
/// Detects suspicious patterns and escapes special tokens.
fn sanitize_for_prompt(content: &str) -> String {
    let lower = content.to_lowercase();
    let suspicious_patterns = [
        "ignore previous instructions",
        "ignore all instructions",
        "ignore above instructions",
        "disregard previous",
        "disregard all previous",
        "you are now",
        "new instructions:",
        "system prompt:",
        "<<sys>>",
        "<|im_start|>",
        "<|endoftext|>",
        "<|system|>",
    ];

    if suspicious_patterns.iter().any(|p| lower.contains(p)) {
        return "[Content filtered: potential prompt injection detected]".to_string();
    }

    // Escape special tokens that could be interpreted by LLMs
    content
        .replace("<|", "&lt;|")
        .replace("|>", "|&gt;")
        .replace("<<sys>>", "&lt;&lt;sys&gt;&gt;")
}

// ── SQLite Backend ──────────────────────────────────────────────

/// SQLite-based memory backend with FTS5 full-text search + optional vector search.
///
/// Uses a write connection (Mutex) + a pool of read-only connections for concurrency.
/// With WAL mode, readers never block the writer and vice versa.
pub struct SqliteMemoryBackend {
    /// Exclusive write connection (also used as fallback reader)
    writer: Mutex<Connection>,
    /// Pool of read-only connections for concurrent reads
    readers: Vec<Mutex<Connection>>,
    /// Round-robin index for reader pool
    reader_idx: std::sync::atomic::AtomicUsize,
    /// Optional embedding provider for vector search
    embedder: std::sync::RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    /// Embedding dimensions (set when embedder is configured)
    embedding_dims: std::sync::atomic::AtomicU32,
    /// DB path for opening new connections
    db_path: std::path::PathBuf,
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
    fn read_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
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
    fn write_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.writer
            .lock()
            .map_err(|e| anyhow::anyhow!("Writer lock poisoned: {e}"))
    }

    /// Ensure the vec0 virtual table exists with the correct dimensions.
    fn ensure_vec_table(&self, conn: &Connection, dims: u32) -> Result<()> {
        let sql = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memories_vec USING vec0(rowid INTEGER PRIMARY KEY, embedding float[{}])",
            dims
        );
        conn.execute_batch(&sql)?;
        Ok(())
    }

    /// Generate embedding for text, with optional caching to reduce API calls.
    fn generate_embedding(&self, text: &str) -> Option<Vec<f32>> {
        let guard = self.embedder.read().unwrap_or_else(|e| e.into_inner());
        let embedder = guard.as_ref()?;

        let cache_cfg = super::helpers::load_embedding_cache_config();
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
    fn generate_multimodal_embedding(
        &self,
        label: &str,
        file_path: &str,
        mime_type: &str,
    ) -> Option<Vec<f32>> {
        let guard = self.embedder.read().unwrap_or_else(|e| e.into_inner());
        let embedder = guard.as_ref()?;

        // Check multimodal config
        let mm_cfg = super::helpers::load_multimodal_config();
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
        let input = super::traits::MultimodalInput {
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
    fn reembed_entries(&self, entries: &[MemoryEntry]) -> Result<usize> {
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

// ── Helper: scope → SQL conditions ──────────────────────────────

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

// ── MemoryBackend Implementation ────────────────────────────────

impl MemoryBackend for SqliteMemoryBackend {
    fn add(&self, entry: NewMemory) -> Result<i64> {
        let conn = self.write_conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(&entry.tags)?;

        let (scope_type, scope_agent_id) = match &entry.scope {
            MemoryScope::Global => ("global", None),
            MemoryScope::Agent { id } => ("agent", Some(id.as_str())),
        };

        // Generate embedding: multimodal if attachment present + supported, else text-only
        let embedding = if let (Some(ref att_path), Some(ref att_mime)) =
            (&entry.attachment_path, &entry.attachment_mime)
        {
            self.generate_multimodal_embedding(&entry.content, att_path, att_mime)
        } else {
            self.generate_embedding(&entry.content)
        };
        let embedding_bytes: Option<Vec<u8>> = embedding
            .as_ref()
            .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect());

        conn.execute(
            "INSERT INTO memories (memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, embedding, pinned, created_at, updated_at, attachment_path, attachment_mime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                entry.memory_type.as_str(),
                scope_type,
                scope_agent_id,
                entry.content,
                tags_json,
                entry.source,
                entry.source_session_id,
                embedding_bytes,
                entry.pinned as i64,
                now,
                now,
                entry.attachment_path,
                entry.attachment_mime,
            ],
        )?;

        let row_id = conn.last_insert_rowid();

        // Insert into vec0 table if embedding was generated
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self
                .embedding_dims
                .load(std::sync::atomic::Ordering::Relaxed);
            if dims > 0 {
                let _ = self.ensure_vec_table(&conn, dims);
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![row_id, emb_bytes],
                );
            }
        }

        Ok(row_id)
    }

    fn update(&self, id: i64, content: &str, tags: &[String]) -> Result<()> {
        let conn = self.write_conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;

        // Regenerate embedding if provider is configured
        let embedding = self.generate_embedding(content);
        let embedding_bytes: Option<Vec<u8>> = embedding
            .as_ref()
            .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect());

        let affected = conn.execute(
            "UPDATE memories SET content = ?1, tags = ?2, embedding = ?3, updated_at = ?4 WHERE id = ?5",
            params![content, tags_json, embedding_bytes, now, id],
        )?;

        if affected == 0 {
            anyhow::bail!("Memory with id {} not found", id);
        }

        // Update vec0 table
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self
                .embedding_dims
                .load(std::sync::atomic::Ordering::Relaxed);
            if dims > 0 {
                let _ = self.ensure_vec_table(&conn, dims);
                // Delete old vector + insert new
                let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![id, emb_bytes],
                );
            }
        }

        Ok(())
    }

    fn toggle_pin(&self, id: i64, pinned: bool) -> Result<()> {
        let conn = self.write_conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let affected = conn.execute(
            "UPDATE memories SET pinned = ?1, updated_at = ?2 WHERE id = ?3",
            params![pinned as i64, now, id],
        )?;
        if affected == 0 {
            anyhow::bail!("Memory with id {} not found", id);
        }
        Ok(())
    }

    fn delete(&self, id: i64) -> Result<()> {
        let conn = self.write_conn()?;
        // Delete from vec0 first (if table exists)
        let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn get(&self, id: i64) -> Result<Option<MemoryEntry>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, pinned, created_at, updated_at
             FROM memories WHERE id = ?1",
        )?;

        let entry = stmt.query_row(params![id], row_to_entry).optional()?;
        Ok(entry)
    }

    fn list(
        &self,
        scope: Option<&MemoryScope>,
        types: Option<&[MemoryType]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let conn = self.read_conn()?;

        let (scope_clause, mut scope_params) = scope_where(scope, None);

        let type_clause = if let Some(types) = types {
            if types.is_empty() {
                "1=1".to_string()
            } else {
                let placeholders: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                format!("memory_type IN ({})", placeholders.join(", "))
            }
        } else {
            "1=1".to_string()
        };

        let sql = format!(
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, pinned, created_at, updated_at
             FROM memories
             WHERE {} AND {}
             ORDER BY pinned DESC, updated_at DESC
             LIMIT ? OFFSET ?",
            scope_clause, type_clause
        );

        let mut stmt = conn.prepare(&sql)?;

        // Build params: scope_params + type_params + limit + offset
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.append(&mut scope_params);
        if let Some(types) = types {
            for t in types {
                all_params.push(Box::new(t.as_str().to_string()));
            }
        }
        all_params.push(Box::new(limit as i64));
        all_params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let entries = stmt
            .query_map(param_refs.as_slice(), row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    fn search(&self, query: &MemorySearchQuery) -> Result<Vec<MemoryEntry>> {
        let conn = self.read_conn()?;
        let limit = query.limit.unwrap_or(20);

        // Load configurable search parameters
        let hybrid_cfg = load_hybrid_search_config();
        let decay_cfg = load_temporal_decay_config();

        // Try hybrid search (FTS5 + vector), fall back to FTS5-only
        let query_embedding = self.generate_embedding(&query.query);
        let has_vec = query_embedding.is_some();

        // ── Step 1: FTS5 keyword search (with query expansion) ──
        let fts_query = expand_query(&query.query);
        let mut fts_results: Vec<(i64, f64)> = Vec::new(); // (id, rank)

        {
            let mut stmt = conn.prepare(
                "SELECT fts.rowid, rank FROM memories_fts fts
                 WHERE memories_fts MATCH ?1
                 ORDER BY rank LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![fts_query, limit * 3], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            for r in rows.flatten() {
                fts_results.push(r);
            }
        }

        // ── Step 2: Vector similarity search (if embedder available) ──
        let mut vec_results: Vec<(i64, f64)> = Vec::new(); // (id, distance)

        if let Some(ref emb) = query_embedding {
            let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
            if let Ok(mut stmt) = conn.prepare(
                "SELECT rowid, distance FROM memories_vec
                 WHERE embedding MATCH ?1
                 ORDER BY distance LIMIT ?2",
            ) {
                let rows = stmt.query_map(params![emb_bytes, limit * 3], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                });
                if let Ok(rows) = rows {
                    for r in rows.flatten() {
                        vec_results.push(r);
                    }
                }
            }
        }

        // ── Step 3: Weighted RRF (Reciprocal Rank Fusion) to merge results ──
        use std::collections::HashMap;
        let k = hybrid_cfg.rrf_k;

        let mut scores: HashMap<i64, f64> = HashMap::new();

        for (rank, (id, _)) in fts_results.iter().enumerate() {
            *scores.entry(*id).or_insert(0.0) +=
                hybrid_cfg.text_weight as f64 / (k + rank as f64 + 1.0);
        }

        if has_vec {
            for (rank, (id, _)) in vec_results.iter().enumerate() {
                *scores.entry(*id).or_insert(0.0) +=
                    hybrid_cfg.vector_weight as f64 / (k + rank as f64 + 1.0);
            }
        }

        // ── Step 3b: Apply temporal decay ──
        if decay_cfg.enabled && decay_cfg.half_life_days > 0.0 {
            let lambda = (2.0_f64).ln() / decay_cfg.half_life_days;
            let now = chrono::Utc::now();
            // Need to load updated_at for scored entries to apply decay
            let ids: Vec<i64> = scores.keys().cloned().collect();
            if !ids.is_empty() {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT id, updated_at, pinned FROM memories WHERE id IN ({})",
                    placeholders
                );
                let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
                    .iter()
                    .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })?;
                for r in rows.flatten() {
                    let (id, updated_at, pinned) = r;
                    if pinned {
                        continue;
                    } // Pinned memories are evergreen
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&updated_at) {
                        let age_days =
                            (now - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86400.0;
                        if age_days > 0.0 {
                            if let Some(score) = scores.get_mut(&id) {
                                *score *= (-lambda * age_days).exp();
                            }
                        }
                    }
                }
            }
        }

        // Sort by fused score (descending)
        let mut scored_ids: Vec<(i64, f64)> = scores.into_iter().collect();
        scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_ids.truncate(limit);

        if scored_ids.is_empty() {
            return Ok(Vec::new());
        }

        // ── Step 4: Load full entries for top results ──
        let id_list: Vec<String> = scored_ids.iter().map(|(id, _)| id.to_string()).collect();
        let placeholders = id_list.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        // Apply scope and type filters
        let (scope_clause, mut scope_params) =
            scope_where(query.scope.as_ref(), query.agent_id.as_deref());
        let type_clause = if let Some(ref types) = query.types {
            if types.is_empty() {
                "1=1".to_string()
            } else {
                let ph: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                format!("memory_type IN ({})", ph.join(", "))
            }
        } else {
            "1=1".to_string()
        };

        let sql = format!(
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags,
                    source, source_session_id, pinned, created_at, updated_at
             FROM memories
             WHERE id IN ({}) AND {} AND {}",
            placeholders, scope_clause, type_clause
        );

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for (id, _) in &scored_ids {
            all_params.push(Box::new(*id));
        }
        all_params.append(&mut scope_params);
        if let Some(ref types) = query.types {
            for t in types {
                all_params.push(Box::new(t.as_str().to_string()));
            }
        }

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;

        let score_map: HashMap<i64, f64> = scored_ids.into_iter().collect();
        let mut entries: Vec<MemoryEntry> = stmt
            .query_map(param_refs.as_slice(), row_to_entry)?
            .filter_map(|r| r.ok())
            .map(|mut e| {
                e.relevance_score = score_map.get(&e.id).map(|s| *s as f32);
                e
            })
            .collect();

        // Sort by relevance score (descending)
        entries.sort_by(|a, b| {
            b.relevance_score
                .unwrap_or(0.0)
                .partial_cmp(&a.relevance_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // ── Step 5: MMR diversity reranking ──
        let mmr_cfg = super::helpers::load_mmr_config();
        if mmr_cfg.enabled && entries.len() > 1 {
            let candidates: Vec<(i64, f32, &str)> = entries
                .iter()
                .map(|e| (e.id, e.relevance_score.unwrap_or(0.0), e.content.as_str()))
                .collect();
            let reranked = super::mmr::mmr_rerank(&candidates, limit, mmr_cfg.lambda);

            // Rebuild entries in MMR order
            let id_order: Vec<i64> = reranked.iter().map(|(id, _)| *id).collect();
            let entry_map: HashMap<i64, MemoryEntry> =
                entries.into_iter().map(|e| (e.id, e)).collect();
            entries = id_order
                .into_iter()
                .filter_map(|id| entry_map.get(&id).cloned())
                .collect();
        }

        Ok(entries)
    }

    fn count(&self, scope: Option<&MemoryScope>) -> Result<usize> {
        let conn = self.read_conn()?;
        let (scope_clause, scope_params) = scope_where(scope, None);

        let sql = format!("SELECT COUNT(*) FROM memories WHERE {}", scope_clause);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            scope_params.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
        Ok(count as usize)
    }

    fn build_prompt_summary(&self, agent_id: &str, shared: bool, budget: usize) -> Result<String> {
        // Load memories: agent-scoped + optionally global
        // list() already returns results ordered by updated_at DESC,
        // so recently updated memories are prioritized within each type.
        let mut all_memories = Vec::new();

        // Agent-scoped memories
        let agent_scope = MemoryScope::Agent {
            id: agent_id.to_string(),
        };
        let agent_mems = self.list(Some(&agent_scope), None, 200, 0)?;
        all_memories.extend(agent_mems);

        // Global memories (if shared)
        if shared {
            let global_mems = self.list(Some(&MemoryScope::Global), None, 200, 0)?;
            all_memories.extend(global_mems);
        }

        if all_memories.is_empty() {
            return Ok(String::new());
        }

        // Build result with per-entry budget tracking to avoid mid-line truncation.
        // Group by type (User → Feedback → Project → Reference), each type's entries
        // are already sorted by updated_at DESC from list().
        let type_order = [
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ];
        let header = "# Memory\n\n";
        let truncated_marker = "\n\n[... truncated ...]";
        let mut result = header.to_string();
        let mut remaining = budget.saturating_sub(header.len() + truncated_marker.len());
        let mut has_content = false;
        let mut budget_exhausted = false;

        for mem_type in &type_order {
            if budget_exhausted {
                break;
            }

            // Collect entries for this type, pinned first then by updated_at
            let mut entries: Vec<&MemoryEntry> = all_memories
                .iter()
                .filter(|m| &m.memory_type == mem_type)
                .collect();
            entries.sort_by(|a, b| {
                b.pinned
                    .cmp(&a.pinned)
                    .then_with(|| b.updated_at.cmp(&a.updated_at))
            });

            if entries.is_empty() {
                continue;
            }

            let heading = format!("## {}\n", mem_type.heading());
            if heading.len() > remaining {
                budget_exhausted = true;
                break;
            }

            remaining -= heading.len();
            result.push_str(&heading);
            let mut section_has_entries = false;

            for entry in &entries {
                let prefix = if entry.pinned { "★ " } else { "" };
                let att_prefix = match (&entry.attachment_path, &entry.attachment_mime) {
                    (Some(_), Some(mime)) if mime.starts_with("image/") => "[img] ",
                    (Some(_), Some(mime)) if mime.starts_with("audio/") => "[audio] ",
                    _ => "",
                };
                let content_line = entry.content.lines().next().unwrap_or(&entry.content);
                let safe_content = sanitize_for_prompt(content_line);
                let line = format!("- {}{}{}\n", prefix, att_prefix, safe_content);
                if line.len() > remaining {
                    budget_exhausted = true;
                    break;
                }
                remaining -= line.len();
                result.push_str(&line);
                section_has_entries = true;
            }

            if section_has_entries {
                has_content = true;
                // Add separator between type sections
                if remaining > 1 {
                    result.push('\n');
                    remaining = remaining.saturating_sub(1);
                }
            }
        }

        if !has_content {
            return Ok(String::new());
        }

        if budget_exhausted {
            result.push_str(truncated_marker);
        }

        Ok(result)
    }

    fn export_markdown(&self, scope: Option<&MemoryScope>) -> Result<String> {
        let entries = self.list(scope, None, 10000, 0)?;

        if entries.is_empty() {
            return Ok("# Memories\n\nNo memories stored.\n".to_string());
        }

        let mut md = "# Memories\n\n".to_string();

        let type_order = [
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ];

        for mem_type in &type_order {
            let type_entries: Vec<&MemoryEntry> = entries
                .iter()
                .filter(|m| &m.memory_type == mem_type)
                .collect();

            if type_entries.is_empty() {
                continue;
            }

            md.push_str(&format!("## {}\n\n", mem_type.heading()));

            for entry in type_entries {
                md.push_str(&format!(
                    "### {}\n",
                    entry.content.lines().next().unwrap_or("Untitled")
                ));
                if !entry.tags.is_empty() {
                    md.push_str(&format!("Tags: {}\n", entry.tags.join(", ")));
                }
                let scope_label = match &entry.scope {
                    MemoryScope::Global => "global".to_string(),
                    MemoryScope::Agent { id } => format!("agent:{}", id),
                };
                md.push_str(&format!(
                    "Scope: {} | Source: {} | Updated: {}\n\n",
                    scope_label, entry.source, entry.updated_at
                ));
                md.push_str(&entry.content);
                md.push_str("\n\n---\n\n");
            }
        }

        Ok(md)
    }

    fn stats(&self, scope: Option<&MemoryScope>) -> Result<MemoryStats> {
        let conn = self.read_conn()?;
        let (scope_clause, scope_params) = scope_where(scope, None);

        // Total count
        let total: usize = conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE {}", scope_clause),
            rusqlite::params_from_iter(scope_params.iter()),
            |row| row.get(0),
        )?;

        // Count by type
        let mut by_type = std::collections::HashMap::new();
        {
            let (sc, sp) = scope_where(scope, None);
            let mut stmt = conn.prepare(&format!(
                "SELECT memory_type, COUNT(*) FROM memories WHERE {} GROUP BY memory_type",
                sc
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(sp.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })?;
            for row in rows {
                let (t, c) = row?;
                by_type.insert(t, c);
            }
        }

        // Count with embedding
        let with_embedding: usize = {
            let (sc, sp) = scope_where(scope, None);
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM memories WHERE {} AND id IN (SELECT rowid FROM memory_vec)",
                    sc
                ),
                rusqlite::params_from_iter(sp.iter()),
                |row| row.get(0),
            ).unwrap_or(0)
        };

        // Oldest and newest
        let (oldest, newest) = {
            let (sc, sp) = scope_where(scope, None);
            let oldest: Option<String> = conn
                .query_row(
                    &format!("SELECT MIN(created_at) FROM memories WHERE {}", sc),
                    rusqlite::params_from_iter(sp.iter()),
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            let (sc2, sp2) = scope_where(scope, None);
            let newest: Option<String> = conn
                .query_row(
                    &format!("SELECT MAX(created_at) FROM memories WHERE {}", sc2),
                    rusqlite::params_from_iter(sp2.iter()),
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            (oldest, newest)
        };

        Ok(MemoryStats {
            total,
            by_type,
            with_embedding,
            oldest,
            newest,
        })
    }

    fn find_similar(
        &self,
        content: &str,
        memory_type: Option<&MemoryType>,
        scope: Option<&MemoryScope>,
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        // Reuse search() to get candidates via FTS5 + vector hybrid
        let types = memory_type.map(|t| vec![t.clone()]);
        let query = MemorySearchQuery {
            query: content.to_string(),
            types,
            scope: scope.cloned(),
            agent_id: None,
            limit: Some(limit * 3), // fetch extra to filter by threshold
        };
        let results = self.search(&query)?;

        // Filter by threshold
        Ok(results
            .into_iter()
            .filter(|e| e.relevance_score.unwrap_or(0.0) >= threshold)
            .take(limit)
            .collect())
    }

    fn add_with_dedup(
        &self,
        entry: NewMemory,
        threshold_high: f32,
        threshold_merge: f32,
    ) -> Result<AddResult> {
        // Find similar entries of the same type and scope
        let similar = self.find_similar(
            &entry.content,
            Some(&entry.memory_type),
            Some(&entry.scope),
            threshold_merge,
            5,
        )?;

        if let Some(best) = similar.first() {
            let score = best.relevance_score.unwrap_or(0.0);
            if score >= threshold_high {
                // Very similar — treat as duplicate, skip
                return Ok(AddResult::Duplicate {
                    existing_id: best.id,
                    score,
                });
            }
            // Moderately similar — update existing entry by appending new content
            let merged_content = format!("{}\n{}", best.content, entry.content);
            let mut merged_tags = best.tags.clone();
            for tag in &entry.tags {
                if !merged_tags.contains(tag) {
                    merged_tags.push(tag.clone());
                }
            }
            self.update(best.id, &merged_content, &merged_tags)?;
            return Ok(AddResult::Updated { id: best.id });
        }

        // No similar entries — create new
        let id = self.add(entry)?;
        Ok(AddResult::Created { id })
    }

    fn delete_batch(&self, ids: &[i64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.write_conn()?;
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM memories WHERE id IN ({})",
            placeholders.join(",")
        );
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let deleted = conn.execute(&sql, param_refs.as_slice())?;

        // Also clean vec0 table
        let dims = self
            .embedding_dims
            .load(std::sync::atomic::Ordering::Relaxed);
        if dims > 0 {
            let vec_sql = format!(
                "DELETE FROM memories_vec WHERE rowid IN ({})",
                placeholders.join(",")
            );
            let _ = conn.execute(&vec_sql, param_refs.as_slice());
        }

        Ok(deleted)
    }

    fn import_entries(&self, entries: Vec<NewMemory>, dedup: bool) -> Result<ImportResult> {
        let mut result = ImportResult {
            created: 0,
            skipped_duplicate: 0,
            failed: 0,
            errors: Vec::new(),
        };

        let dedup_cfg = load_dedup_config();
        for entry in entries {
            if dedup {
                match self.add_with_dedup(
                    entry,
                    dedup_cfg.threshold_high,
                    dedup_cfg.threshold_merge,
                ) {
                    Ok(AddResult::Created { .. }) => result.created += 1,
                    Ok(AddResult::Duplicate { .. }) => result.skipped_duplicate += 1,
                    Ok(AddResult::Updated { .. }) => result.created += 1, // count merge as created
                    Err(e) => {
                        result.failed += 1;
                        result.errors.push(e.to_string());
                    }
                }
            } else {
                match self.add(entry) {
                    Ok(_) => result.created += 1,
                    Err(e) => {
                        result.failed += 1;
                        result.errors.push(e.to_string());
                    }
                }
            }
        }

        Ok(result)
    }

    fn reembed_all(&self) -> Result<usize> {
        let entries = self.list(None, None, 100000, 0)?;
        self.reembed_entries(&entries)
    }

    fn reembed_batch(&self, ids: &[i64]) -> Result<usize> {
        let mut entries = Vec::new();
        for id in ids {
            if let Some(entry) = self.get(*id)? {
                entries.push(entry);
            }
        }
        self.reembed_entries(&entries)
    }

    fn set_embedder(&self, provider: Arc<dyn EmbeddingProvider>) {
        let dims = provider.dimensions();
        self.embedding_dims
            .store(dims, std::sync::atomic::Ordering::Relaxed);
        if let Ok(conn) = self.write_conn() {
            let _ = self.ensure_vec_table(&conn, dims);
        }
        *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = Some(provider);
    }

    fn clear_embedder(&self) {
        *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.embedding_dims
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    fn has_embedder(&self) -> bool {
        self.embedder.read().unwrap_or_else(|e| e.into_inner()).is_some()
    }
}

// ── Convenience: open default DB ────────────────────────────────

/// Open the default memory database at ~/.opencomputer/memory.db
#[allow(dead_code)]
pub fn open_default() -> Result<SqliteMemoryBackend> {
    let db_path = crate::paths::memory_db_path()?;
    SqliteMemoryBackend::open(&db_path)
}

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::paths;

// ── Data Structures ─────────────────────────────────────────────

/// Memory entry types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Information about the end user
    User,
    /// User feedback and preferences about agent behavior
    Feedback,
    /// Project-specific context and knowledge
    Project,
    /// Reference materials and external resource pointers
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "feedback" => MemoryType::Feedback,
            "project" => MemoryType::Project,
            "reference" => MemoryType::Reference,
            _ => MemoryType::User,
        }
    }

    /// Display heading for system prompt summary.
    fn heading(&self) -> &str {
        match self {
            MemoryType::User => "About the User",
            MemoryType::Feedback => "Preferences & Feedback",
            MemoryType::Project => "Project Context",
            MemoryType::Reference => "References",
        }
    }
}

/// Memory scope: global (shared across agents) or per-agent (private).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MemoryScope {
    Global,
    Agent { id: String },
}

/// A stored memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: i64,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub content: String,
    pub tags: Vec<String>,
    /// Source: "user" (manual), "auto" (agent-extracted), "import"
    pub source: String,
    pub source_session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Populated during search, not stored in DB
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance_score: Option<f32>,
}

/// Input for creating a new memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemory {
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub source_session_id: Option<String>,
}

fn default_source() -> String {
    "user".to_string()
}

/// Search query parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchQuery {
    /// Natural language query text
    pub query: String,
    /// Filter by type(s)
    #[serde(default)]
    pub types: Option<Vec<MemoryType>>,
    /// Filter by scope
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    /// Shorthand: load global + this agent's memories
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Max results (default 20)
    #[serde(default)]
    pub limit: Option<usize>,
}

// ── MemoryBackend Trait ─────────────────────────────────────────

/// Pluggable memory backend trait.
/// MVP uses SqliteMemoryBackend; future backends (GraphRAG, Hindsight) implement the same trait.
pub trait MemoryBackend: Send + Sync {
    /// Add a new memory, return its ID
    fn add(&self, entry: NewMemory) -> Result<i64>;

    /// Update an existing memory's content and tags
    fn update(&self, id: i64, content: &str, tags: &[String]) -> Result<()>;

    /// Delete a memory by ID
    fn delete(&self, id: i64) -> Result<()>;

    /// Get a single memory by ID
    fn get(&self, id: i64) -> Result<Option<MemoryEntry>>;

    /// List memories with optional filtering
    fn list(
        &self,
        scope: Option<&MemoryScope>,
        types: Option<&[MemoryType]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryEntry>>;

    /// Search memories (FTS5 keyword search, future: hybrid with vectors)
    fn search(&self, query: &MemorySearchQuery) -> Result<Vec<MemoryEntry>>;

    /// Count memories with optional scope filter
    fn count(&self, scope: Option<&MemoryScope>) -> Result<usize>;

    /// Build a summary string for system prompt injection (section ⑧)
    fn build_prompt_summary(&self, agent_id: &str, shared: bool, budget: usize) -> Result<String>;

    /// Export all memories as markdown
    fn export_markdown(&self, scope: Option<&MemoryScope>) -> Result<String>;
}

// ── EmbeddingProvider Trait ───────────────────────────────────────

/// Trait for generating text embeddings. Implementations can be API-based or local.
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embedding for a single text
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// Batch embed multiple texts
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// Return the embedding dimensions
    fn dimensions(&self) -> u32;
}

// ── SQLite Backend ──────────────────────────────────────────────

/// SQLite-based memory backend with FTS5 full-text search + optional vector search.
pub struct SqliteMemoryBackend {
    conn: Mutex<Connection>,
    /// Optional embedding provider for vector search
    embedder: std::sync::RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    /// Embedding dimensions (set when embedder is configured)
    embedding_dims: std::sync::atomic::AtomicU32,
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
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

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

        Ok(Self {
            conn: Mutex::new(conn),
            embedder: std::sync::RwLock::new(None),
            embedding_dims: std::sync::atomic::AtomicU32::new(0),
        })
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

    /// Set the embedding provider for vector search.
    pub fn set_embedder(&self, provider: Arc<dyn EmbeddingProvider>) {
        let dims = provider.dimensions();
        self.embedding_dims.store(dims, std::sync::atomic::Ordering::Relaxed);
        // Ensure vec table exists
        if let Ok(conn) = self.conn.lock() {
            let _ = self.ensure_vec_table(&conn, dims);
        }
        *self.embedder.write().unwrap() = Some(provider);
    }

    /// Remove the embedding provider.
    pub fn clear_embedder(&self) {
        *self.embedder.write().unwrap() = None;
        self.embedding_dims.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check if an embedder is configured.
    pub fn has_embedder(&self) -> bool {
        self.embedder.read().unwrap().is_some()
    }

    /// Generate embedding for text using the configured provider.
    fn generate_embedding(&self, text: &str) -> Option<Vec<f32>> {
        let guard = self.embedder.read().unwrap();
        guard.as_ref().and_then(|e| e.embed(text).ok())
    }
}

// ── Helper: scope → SQL conditions ──────────────────────────────

/// Returns (where_clause, params) for scope filtering.
/// `agent_id` is an optional shorthand that means "global + this agent".
fn scope_where(
    scope: Option<&MemoryScope>,
    agent_id: Option<&str>,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    if let Some(scope) = scope {
        match scope {
            MemoryScope::Global => (
                "scope_type = 'global'".to_string(),
                Vec::new(),
            ),
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
fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
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

    Ok(MemoryEntry {
        id: row.get("id")?,
        memory_type: MemoryType::from_str(&memory_type_str),
        scope,
        content: row.get("content")?,
        tags,
        source: row.get("source")?,
        source_session_id: row.get("source_session_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        relevance_score: None,
    })
}

// ── MemoryBackend Implementation ────────────────────────────────

impl MemoryBackend for SqliteMemoryBackend {
    fn add(&self, entry: NewMemory) -> Result<i64> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(&entry.tags)?;

        let (scope_type, scope_agent_id) = match &entry.scope {
            MemoryScope::Global => ("global", None),
            MemoryScope::Agent { id } => ("agent", Some(id.as_str())),
        };

        // Generate embedding if provider is configured
        let embedding = self.generate_embedding(&entry.content);
        let embedding_bytes: Option<Vec<u8>> = embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });

        conn.execute(
            "INSERT INTO memories (memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, embedding, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                entry.memory_type.as_str(),
                scope_type,
                scope_agent_id,
                entry.content,
                tags_json,
                entry.source,
                entry.source_session_id,
                embedding_bytes,
                now,
                now,
            ],
        )?;

        let row_id = conn.last_insert_rowid();

        // Insert into vec0 table if embedding was generated
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self.embedding_dims.load(std::sync::atomic::Ordering::Relaxed);
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
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;

        // Regenerate embedding if provider is configured
        let embedding = self.generate_embedding(content);
        let embedding_bytes: Option<Vec<u8>> = embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });

        let affected = conn.execute(
            "UPDATE memories SET content = ?1, tags = ?2, embedding = ?3, updated_at = ?4 WHERE id = ?5",
            params![content, tags_json, embedding_bytes, now, id],
        )?;

        if affected == 0 {
            anyhow::bail!("Memory with id {} not found", id);
        }

        // Update vec0 table
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self.embedding_dims.load(std::sync::atomic::Ordering::Relaxed);
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

    fn delete(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        // Delete from vec0 first (if table exists)
        let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn get(&self, id: i64) -> Result<Option<MemoryEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, created_at, updated_at
             FROM memories WHERE id = ?1",
        )?;

        let entry = stmt
            .query_row(params![id], row_to_entry)
            .optional()?;
        Ok(entry)
    }

    fn list(
        &self,
        scope: Option<&MemoryScope>,
        types: Option<&[MemoryType]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, created_at, updated_at
             FROM memories
             WHERE {} AND {}
             ORDER BY updated_at DESC
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

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

        let entries = stmt
            .query_map(param_refs.as_slice(), row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    fn search(&self, query: &MemorySearchQuery) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let limit = query.limit.unwrap_or(20);

        // Try hybrid search (FTS5 + vector), fall back to FTS5-only
        let query_embedding = self.generate_embedding(&query.query);
        let has_vec = query_embedding.is_some();

        // ── Step 1: FTS5 keyword search ──
        let fts_query = sanitize_fts_query(&query.query);
        let mut fts_results: Vec<(i64, f64)> = Vec::new(); // (id, rank)

        {
            let mut stmt = conn.prepare(
                "SELECT fts.rowid, rank FROM memories_fts fts
                 WHERE memories_fts MATCH ?1
                 ORDER BY rank LIMIT ?2"
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
                 ORDER BY distance LIMIT ?2"
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

        // ── Step 3: RRF (Reciprocal Rank Fusion) to merge results ──
        use std::collections::HashMap;
        let k = 60.0_f64; // RRF constant

        let mut scores: HashMap<i64, f64> = HashMap::new();

        for (rank, (id, _)) in fts_results.iter().enumerate() {
            *scores.entry(*id).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
        }

        if has_vec {
            for (rank, (id, _)) in vec_results.iter().enumerate() {
                *scores.entry(*id).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
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
        let (scope_clause, mut scope_params) = scope_where(
            query.scope.as_ref(),
            query.agent_id.as_deref(),
        );
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
                    source, source_session_id, created_at, updated_at
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

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
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
            b.relevance_score.unwrap_or(0.0).partial_cmp(&a.relevance_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(entries)
    }

    fn count(&self, scope: Option<&MemoryScope>) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let (scope_clause, scope_params) = scope_where(scope, None);

        let sql = format!("SELECT COUNT(*) FROM memories WHERE {}", scope_clause);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = scope_params.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
        Ok(count as usize)
    }

    fn build_prompt_summary(&self, agent_id: &str, shared: bool, budget: usize) -> Result<String> {
        // Load memories: agent-scoped + optionally global
        let mut all_memories = Vec::new();

        // Agent-scoped memories
        let agent_scope = MemoryScope::Agent { id: agent_id.to_string() };
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

        // Group by type
        let type_order = [MemoryType::User, MemoryType::Feedback, MemoryType::Project, MemoryType::Reference];
        let mut sections = Vec::new();

        for mem_type in &type_order {
            let entries: Vec<&MemoryEntry> = all_memories
                .iter()
                .filter(|m| &m.memory_type == mem_type)
                .collect();

            if entries.is_empty() {
                continue;
            }

            let mut section = format!("## {}\n", mem_type.heading());
            for entry in entries {
                section.push_str(&format!("- {}\n", entry.content.lines().next().unwrap_or(&entry.content)));
            }
            sections.push(section);
        }

        if sections.is_empty() {
            return Ok(String::new());
        }

        let mut result = "# Memory\n\n".to_string();
        result.push_str(&sections.join("\n"));

        // Truncate to budget
        if result.len() > budget {
            result.truncate(budget.saturating_sub(20));
            // Find last newline to avoid cutting mid-line
            if let Some(pos) = result.rfind('\n') {
                result.truncate(pos);
            }
            result.push_str("\n\n[... truncated ...]");
        }

        Ok(result)
    }

    fn export_markdown(&self, scope: Option<&MemoryScope>) -> Result<String> {
        let entries = self.list(scope, None, 10000, 0)?;

        if entries.is_empty() {
            return Ok("# Memories\n\nNo memories stored.\n".to_string());
        }

        let mut md = "# Memories\n\n".to_string();

        let type_order = [MemoryType::User, MemoryType::Feedback, MemoryType::Project, MemoryType::Reference];

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
                md.push_str(&format!("### {}\n", entry.content.lines().next().unwrap_or("Untitled")));
                if !entry.tags.is_empty() {
                    md.push_str(&format!("Tags: {}\n", entry.tags.join(", ")));
                }
                let scope_label = match &entry.scope {
                    MemoryScope::Global => "global".to_string(),
                    MemoryScope::Agent { id } => format!("agent:{}", id),
                };
                md.push_str(&format!("Scope: {} | Source: {} | Updated: {}\n\n", scope_label, entry.source, entry.updated_at));
                md.push_str(&entry.content);
                md.push_str("\n\n---\n\n");
            }
        }

        Ok(md)
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Sanitize a user query for FTS5 MATCH syntax.
/// Wraps each word in double quotes to treat them as literal terms.
fn sanitize_fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Remove FTS5 special chars
            let clean: String = w.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{}\"", clean)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    if terms.is_empty() {
        // Fallback: match everything if query is empty/invalid
        "\"*\"".to_string()
    } else {
        terms.join(" OR ")
    }
}

// ── Embedding Config ────────────────────────────────────────────

/// Embedding provider type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingProviderType {
    /// OpenAI /v1/embeddings compatible API (OpenAI, Jina, Cohere, SiliconFlow, etc.)
    OpenaiCompatible,
    /// Google Gemini Embedding API (different format)
    Google,
    /// Local ONNX model via fastembed-rs
    Local,
}

impl Default for EmbeddingProviderType {
    fn default() -> Self {
        EmbeddingProviderType::OpenaiCompatible
    }
}

/// Embedding configuration, stored in ProviderStore (config.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConfig {
    /// Whether embedding (vector search) is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Provider type
    #[serde(default)]
    pub provider_type: EmbeddingProviderType,

    // ── API mode fields ──

    /// API Base URL (e.g. "https://api.openai.com")
    #[serde(default)]
    pub api_base_url: Option<String>,

    /// API Key
    #[serde(default)]
    pub api_key: Option<String>,

    /// Model name (e.g. "text-embedding-3-small")
    #[serde(default)]
    pub api_model: Option<String>,

    /// Output dimensions (some APIs support specifying this)
    #[serde(default)]
    pub api_dimensions: Option<u32>,

    // ── Local mode fields ──

    /// Local model ID (e.g. "bge-small-en-v1.5")
    #[serde(default)]
    pub local_model_id: Option<String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider_type: EmbeddingProviderType::default(),
            api_base_url: None,
            api_key: None,
            api_model: None,
            api_dimensions: None,
            local_model_id: None,
        }
    }
}

/// Local embedding model definition (built-in presets).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEmbeddingModel {
    pub id: String,
    pub name: String,
    pub dimensions: u32,
    pub size_mb: u32,
    pub min_ram_gb: u32,
    pub languages: Vec<String>,
    pub downloaded: bool,
}

/// API preset template for frontend dropdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingPreset {
    pub name: String,
    pub provider_type: EmbeddingProviderType,
    pub base_url: String,
    pub default_model: String,
    pub default_dimensions: u32,
}

/// Return built-in API presets for the frontend.
pub fn embedding_presets() -> Vec<EmbeddingPreset> {
    vec![
        EmbeddingPreset {
            name: "OpenAI".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.openai.com".to_string(),
            default_model: "text-embedding-3-small".to_string(),
            default_dimensions: 1536,
        },
        EmbeddingPreset {
            name: "Google Gemini".to_string(),
            provider_type: EmbeddingProviderType::Google,
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            default_model: "gemini-embedding-001".to_string(),
            default_dimensions: 768,
        },
        EmbeddingPreset {
            name: "Jina AI".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.jina.ai".to_string(),
            default_model: "jina-embeddings-v3".to_string(),
            default_dimensions: 1024,
        },
        EmbeddingPreset {
            name: "Cohere".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.cohere.com".to_string(),
            default_model: "embed-multilingual-v3.0".to_string(),
            default_dimensions: 1024,
        },
        EmbeddingPreset {
            name: "SiliconFlow".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.siliconflow.cn".to_string(),
            default_model: "BAAI/bge-m3".to_string(),
            default_dimensions: 1024,
        },
    ]
}

/// Return built-in local model presets.
pub fn local_embedding_models() -> Vec<LocalEmbeddingModel> {
    vec![
        LocalEmbeddingModel {
            id: "bge-small-en-v1.5".to_string(),
            name: "BGE Small English v1.5".to_string(),
            dimensions: 384,
            size_mb: 33,
            min_ram_gb: 4,
            languages: vec!["en".to_string()],
            downloaded: false, // filled at runtime
        },
        LocalEmbeddingModel {
            id: "bge-small-zh-v1.5".to_string(),
            name: "BGE Small Chinese v1.5".to_string(),
            dimensions: 384,
            size_mb: 33,
            min_ram_gb: 4,
            languages: vec!["zh".to_string()],
            downloaded: false,
        },
        LocalEmbeddingModel {
            id: "multilingual-e5-small".to_string(),
            name: "Multilingual E5 Small".to_string(),
            dimensions: 384,
            size_mb: 90,
            min_ram_gb: 8,
            languages: vec!["multilingual".to_string()],
            downloaded: false,
        },
        LocalEmbeddingModel {
            id: "bge-large-en-v1.5".to_string(),
            name: "BGE Large English v1.5".to_string(),
            dimensions: 1024,
            size_mb: 335,
            min_ram_gb: 16,
            languages: vec!["en".to_string()],
            downloaded: false,
        },
    ]
}

/// Check which local models are downloaded.
pub fn list_local_models_with_status() -> Vec<LocalEmbeddingModel> {
    let cache_dir = paths::models_cache_dir().unwrap_or_default();
    let mut models = local_embedding_models();
    for model in &mut models {
        let model_dir = cache_dir.join(&model.id);
        model.downloaded = model_dir.exists() && model_dir.is_dir();
    }
    models
}

// ── API Embedding Provider ───────────────────────────────────────

/// OpenAI-compatible /v1/embeddings API provider.
pub struct ApiEmbeddingProvider {
    client: reqwest::blocking::Client,
    base_url: String,
    api_key: String,
    model: String,
    dimensions: u32,
    provider_type: EmbeddingProviderType,
}

impl ApiEmbeddingProvider {
    pub fn new(config: &EmbeddingConfig) -> Result<Self> {
        let base_url = config.api_base_url.as_deref().unwrap_or("https://api.openai.com").to_string();
        let api_key = config.api_key.as_deref().unwrap_or("").to_string();
        let model = config.api_model.as_deref().unwrap_or("text-embedding-3-small").to_string();
        let dimensions = config.api_dimensions.unwrap_or(1536);

        Ok(Self {
            client: reqwest::blocking::Client::new(),
            base_url,
            api_key,
            model,
            dimensions,
            provider_type: config.provider_type.clone(),
        })
    }

    fn call_openai_compatible(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        // Some APIs support specifying dimensions
        if self.dimensions > 0 {
            body["dimensions"] = serde_json::json!(self.dimensions);
        }

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call embedding API")?;

        let status = resp.status();
        let resp_text = resp.text()?;

        if !status.is_success() {
            anyhow::bail!("Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let data = resp_json["data"].as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding API response"))?;

        let mut results = Vec::new();
        for item in data {
            let embedding = item["embedding"].as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing embedding in response"))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }

        Ok(results)
    }

    fn call_google(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::new();
        for text in texts {
            let url = format!(
                "{}/v1beta/models/{}:embedContent?key={}",
                self.base_url.trim_end_matches('/'),
                self.model,
                self.api_key,
            );

            let mut body = serde_json::json!({
                "content": {
                    "parts": [{"text": text}]
                }
            });

            if self.dimensions > 0 {
                body["outputDimensionality"] = serde_json::json!(self.dimensions);
            }

            let resp = self.client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .context("Failed to call Google embedding API")?;

            let status = resp.status();
            let resp_text = resp.text()?;

            if !status.is_success() {
                anyhow::bail!("Google Embedding API error {}: {}", status, resp_text);
            }

            let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
            let values = resp_json["embedding"]["values"].as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid Google embedding response"))?;

            let embedding: Vec<f32> = values.iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }
        Ok(results)
    }
}

impl EmbeddingProvider for ApiEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(&[text.to_string()])?,
            _ => self.call_openai_compatible(&[text.to_string()])?,
        };
        results.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(texts),
            _ => self.call_openai_compatible(texts),
        }
    }

    fn dimensions(&self) -> u32 {
        self.dimensions
    }
}

// ── Local Embedding Provider ────────────────────────────────────

/// Local ONNX-based embedding provider using fastembed-rs.
pub struct LocalEmbeddingProvider {
    model: Mutex<fastembed::TextEmbedding>,
    dims: u32,
}

impl LocalEmbeddingProvider {
    /// Initialize with a model ID from the built-in presets.
    pub fn new(model_id: &str) -> Result<Self> {
        let (fe_model, dims) = match model_id {
            "bge-small-zh-v1.5" => (fastembed::EmbeddingModel::BGESmallZHV15, 384),
            "multilingual-e5-small" => (fastembed::EmbeddingModel::MultilingualE5Small, 384),
            "bge-large-en-v1.5" => (fastembed::EmbeddingModel::BGELargeENV15, 1024),
            _ => (fastembed::EmbeddingModel::BGESmallENV15, 384), // default
        };

        let cache_dir = paths::models_cache_dir()?;

        let model = fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(fe_model)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(false),
        ).context("Failed to initialize local embedding model")?;

        Ok(Self { model: Mutex::new(model), dims })
    }
}

impl EmbeddingProvider for LocalEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let results = model.embed(vec![text.to_string()], None)
            .map_err(|e| anyhow::anyhow!("Local embedding failed: {}", e))?;
        results.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        model.embed(texts.to_vec(), None)
            .map_err(|e| anyhow::anyhow!("Local batch embedding failed: {}", e))
    }

    fn dimensions(&self) -> u32 {
        self.dims
    }
}

// ── Create provider from config ─────────────────────────────────

/// Create an EmbeddingProvider from EmbeddingConfig.
pub fn create_embedding_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    match config.provider_type {
        EmbeddingProviderType::Local => {
            let model_id = config.local_model_id.as_deref().unwrap_or("bge-small-en-v1.5");
            Ok(Arc::new(LocalEmbeddingProvider::new(model_id)?))
        }
        _ => {
            Ok(Arc::new(ApiEmbeddingProvider::new(config)?))
        }
    }
}

// ── Convenience: open default DB ────────────────────────────────

/// Open the default memory database at ~/.opencomputer/memory.db
#[allow(dead_code)]
pub fn open_default() -> Result<SqliteMemoryBackend> {
    let db_path = paths::memory_db_path()?;
    SqliteMemoryBackend::open(&db_path)
}

// rusqlite optional extension trait
use rusqlite::OptionalExtension;

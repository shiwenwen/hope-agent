use anyhow::Result;
use std::sync::Arc;

use super::types::*;

// ── MemoryBackend Trait ─────────────────────────────────────────

/// Pluggable memory backend.
///
/// The default implementation is [`super::sqlite::SqliteMemoryBackend`] — a
/// local SQLite + FTS5 + optional vector index. Alternative backends (remote
/// services like Honcho, dedicated vector stores like QMD, or bespoke user
/// backends) can be plugged in by implementing this trait.
///
/// ## Implementing a new backend — checklist
///
/// 1. **State**: create a struct holding whatever state your backend needs
///    (HTTP client, cached auth token, local index files, …). Keep it `Send
///    + Sync`; wrap interior mutability in `Mutex` / `RwLock` as needed.
/// 2. **CRUD methods** (required): implement `add` / `update` / `delete` /
///    `get` / `list` / `count` / `stats` / `search`. Return [`anyhow::Error`]
///    on failure — callers log and degrade gracefully.
/// 3. **Scope semantics**: honour [`MemoryScope::Global`] / `Agent { id }` /
///    `Project { id }` in every method that takes a scope. The system prompt
///    and Active Memory recall pipelines rely on scope isolation.
/// 4. **Prompt injection**: implement `build_prompt_summary` and optionally
///    override `build_prompt_summary_with_project` to give project-scoped
///    memories precedence. Keep the output budget (chars) strictly under
///    `budget` — the caller slots this directly into the system prompt.
/// 5. **Search**: `search` must handle an empty query string gracefully
///    (return recent entries) since Active Memory sometimes passes raw user
///    text. Populate `MemoryEntry.relevance_score` when your backend can.
/// 6. **Dedup** (optional but recommended): implement `find_similar` and
///    `add_with_dedup` — the auto-extraction pipeline uses these to avoid
///    storing near-duplicates. The default `add_with_dedup` is unaware of
///    your similarity metric, so it's worth overriding.
/// 7. **Embeddings** (optional): if your backend can store vectors,
///    override `set_embedder` / `has_embedder` / `reembed_all` /
///    `reembed_batch`. Remote backends usually manage embeddings internally
///    and can leave these as no-ops.
/// 8. **Lifecycle** (optional): implement `sync` if your backend has an
///    explicit refresh step (pulling new entries from a remote service).
///    The default is a no-op so local backends don't need to care.
/// 9. **Identity**: override `backend_kind()` with a short lowercase tag
///    (e.g. `"honcho"`, `"qmd"`). This is used in logs and Dashboard
///    labels; don't reuse `"sqlite"`.
/// 10. **Wire it up**: call `crate::set_memory_backend(Arc::new(YourBackend))`
///     during app init before the first `AssistantAgent::chat()`. After that
///     `crate::get_memory_backend()` returns your backend and every caller
///     (system prompt, Active Memory, auto-extract, tools) uses it.
///
/// Most trait methods are synchronous because the reference implementation
/// talks to a local SQLite file and the async overhead is not worth it for
/// that case. When a future remote backend needs async I/O, wrap the
/// blocking portion in `tokio::task::spawn_blocking` inside the method body
/// — the call sites are already invoked from async contexts via
/// `spawn_blocking` in [`crate::agent::active_memory`] and the extraction
/// pipeline, so throughput is preserved.
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

    /// Load candidate memories for prompt injection (agent + optionally global).
    /// Used by LLM memory selection to get raw entries before filtering.
    fn load_prompt_candidates(&self, agent_id: &str, shared: bool) -> Result<Vec<MemoryEntry>>;

    /// Build a summary string for prompt injection, including an optional
    /// [`MemoryScope::Project`] source when the current session belongs to a
    /// project. Project memories take precedence in the returned ordering so
    /// budget-based truncation preserves project context first.
    ///
    /// Default implementation delegates to [`Self::build_prompt_summary`] and
    /// ignores the project id, so backends can opt in incrementally.
    fn build_prompt_summary_with_project(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        shared: bool,
        budget: usize,
    ) -> Result<String> {
        let _ = project_id;
        self.build_prompt_summary(agent_id, shared, budget)
    }

    /// Load candidate memories including project-scoped entries.
    ///
    /// Ordering priority: **Project** → Agent → Global. Pinned entries still
    /// float to the top within their type group via `format_prompt_summary`.
    ///
    /// Default implementation delegates to [`Self::load_prompt_candidates`]
    /// and ignores the project id.
    fn load_prompt_candidates_with_project(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        shared: bool,
    ) -> Result<Vec<MemoryEntry>> {
        let _ = project_id;
        self.load_prompt_candidates(agent_id, shared)
    }

    /// Count memories assigned to a specific project scope. Used by the
    /// project list endpoint to display per-project memory counts.
    fn count_by_project(&self, project_id: &str) -> Result<usize> {
        self.count(Some(&MemoryScope::Project {
            id: project_id.to_string(),
        }))
    }

    /// Export all memories as markdown
    fn export_markdown(&self, scope: Option<&MemoryScope>) -> Result<String>;

    /// Get memory statistics
    fn stats(&self, scope: Option<&MemoryScope>) -> Result<MemoryStats>;

    // ── Pin ──

    /// Toggle the pinned status of a memory.
    fn toggle_pin(&self, id: i64, pinned: bool) -> Result<()>;

    // ── Deduplication ──

    /// Find memories similar to the given content (for dedup checks).
    /// Returns entries above the threshold score, sorted by relevance descending.
    fn find_similar(
        &self,
        content: &str,
        memory_type: Option<&MemoryType>,
        scope: Option<&MemoryScope>,
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>>;

    /// Add a memory with deduplication: skips if very similar, updates if moderately similar.
    fn add_with_dedup(
        &self,
        entry: NewMemory,
        threshold_high: f32,
        threshold_merge: f32,
    ) -> Result<AddResult>;

    // ── Batch operations ──

    /// Delete multiple memories by ID. Returns the number deleted.
    fn delete_batch(&self, ids: &[i64]) -> Result<usize>;

    /// Return every distinct `scope_project_id` value present on rows whose
    /// `scope_type = 'project'`. Used by [`crate::project::reconcile`] at
    /// startup to find orphan project-scoped memory rows whose owning project
    /// row has already been deleted (the cross-database delete path is
    /// non-transactional — see `project/files.rs::delete_project_cascade`).
    ///
    /// Default impl returns `Ok(vec![])` so out-of-tree backends without
    /// project-scope support stay sound.
    fn list_distinct_project_scope_ids(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Import multiple memories with optional deduplication.
    fn import_entries(&self, entries: Vec<NewMemory>, dedup: bool) -> Result<ImportResult>;

    /// Regenerate embeddings for all memories (or those missing embeddings).
    fn reembed_all(&self) -> Result<usize>;

    /// Regenerate embeddings for specific memories.
    fn reembed_batch(&self, ids: &[i64]) -> Result<usize>;

    // ── Embedder management (default no-op for backends without vector support) ──

    /// Set the embedding provider for vector search.
    fn set_embedder(&self, _provider: Arc<dyn EmbeddingProvider>) {}

    /// Remove the embedding provider.
    fn clear_embedder(&self) {}

    /// Check if an embedder is configured.
    fn has_embedder(&self) -> bool {
        false
    }

    // ── Backend identity & lifecycle ──

    /// Short lowercase tag identifying the backend implementation.
    /// Used in logs and Dashboard labels. Each implementation should return
    /// a distinct value (`"sqlite"`, `"honcho"`, `"qmd"`, …).
    fn backend_kind(&self) -> &'static str {
        "unknown"
    }

    /// Explicit refresh hook for remote backends (no-op for local stores).
    /// Called by the Dashboard "refresh" action and by scheduled sync jobs
    /// when the backend kind advertises remote state. Return quickly;
    /// long-running syncs should be dispatched internally.
    fn sync(&self) -> Result<()> {
        Ok(())
    }

    /// Whether this backend exposes vector search via `search` in addition
    /// to keyword / FTS. Callers use this to decide whether to ask for
    /// vector-scored ranking vs. plain text ranking. The default returns
    /// `true` iff an embedder is configured.
    fn supports_vectors(&self) -> bool {
        self.has_embedder()
    }

    /// Phase B'2 — count entries tagged `profile` (reflective memories)
    /// created within `window_days`. Default returns 0; local backends
    /// override with an efficient SQL count.
    fn count_profile_memories(&self, _window_days: u32) -> Result<u64> {
        Ok(0)
    }
}

// ── EmbeddingProvider Trait ───────────────────────────────────────

/// Input for multimodal embedding: text label + binary file data.
pub struct MultimodalInput {
    /// Descriptive label, e.g. "Image file: photo.jpg"
    pub label: String,
    /// MIME type, e.g. "image/jpeg"
    pub mime_type: String,
    /// Raw file bytes (will be base64-encoded for API calls)
    pub file_data: Vec<u8>,
}

/// Trait for generating text embeddings. Implementations can be API-based or local.
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embedding for a single text
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// Batch embed multiple texts
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// Return the embedding dimensions
    fn dimensions(&self) -> u32;

    /// Whether this provider supports multimodal embedding (image/audio → vector).
    /// Only Gemini embedding-2-preview supports this.
    fn supports_multimodal(&self) -> bool {
        false
    }

    /// Generate embedding for a multimodal input (text + image/audio file).
    /// Default: falls back to text-only embedding of the label.
    fn embed_multimodal(&self, input: &MultimodalInput) -> Result<Vec<f32>> {
        self.embed(&input.label)
    }

    /// Whether this provider supports the async Batch API (JSONL upload → poll → download).
    /// Used for bulk re-embedding at ~50% lower cost.
    fn supports_batch_api(&self) -> bool {
        false
    }

    /// Submit a batch embedding job via the async Batch API.
    /// Returns a map of custom_id → embedding vector.
    /// Default: falls back to synchronous embed_batch().
    fn embed_batch_async(
        &self,
        texts: &[(String, String)],
    ) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        // Default: synchronous fallback
        let text_strs: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
        let results = self.embed_batch(&text_strs)?;
        let mut map = std::collections::HashMap::new();
        for ((id, _), emb) in texts.iter().zip(results) {
            map.insert(id.clone(), emb);
        }
        Ok(map)
    }
}

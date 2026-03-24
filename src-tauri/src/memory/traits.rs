use anyhow::Result;
use std::sync::Arc;

use super::types::*;

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

    /// Get memory statistics
    fn stats(&self, scope: Option<&MemoryScope>) -> Result<MemoryStats>;

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
    fn add_with_dedup(&self, entry: NewMemory, threshold_high: f32, threshold_merge: f32) -> Result<AddResult>;

    // ── Batch operations ──

    /// Delete multiple memories by ID. Returns the number deleted.
    fn delete_batch(&self, ids: &[i64]) -> Result<usize>;

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

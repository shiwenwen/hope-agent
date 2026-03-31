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
